# Contracts of feature 028 (drafts)

Draft fragments, merged into the canonical contracts by T028-005 at implementation. None is in force.

| Draft | Merges into | Kind of change |
|---|---|---|
| [testing.openapi.yaml](testing.openapi.yaml) | `specs/001-platform-foundation/contracts/testing.openapi.yaml` (new document, test server only) | new |
| [git-sync.openapi.yaml](git-sync.openapi.yaml) | `specs/001-platform-foundation/contracts/openapi.yaml` | additive paths and schemas |
| [templates.openapi.yaml](templates.openapi.yaml) | `specs/001-platform-foundation/contracts/openapi.yaml` | additive paths and schemas |
| [time-classes.md](time-classes.md) | `crates/infra-postgres/time-classification.json` and `docs/architecture/execution-semantics.md`; cited by every draft that adds an instant (decision C13) | new catalog |
| [errors.md](errors.md) | `openapi.yaml` (`PlatformVersion.capabilities`, reason lists), `sdks/error-catalog.json` (v2), `crates/domain/src/identity.rs` | additive |
| [sdk-interceptors.md](sdk-interceptors.md) | `sdks/SPEC.md` §12 | new section |
| [test-environment-sdk.md](test-environment-sdk.md) | `sdks/SPEC.md` §13 | new section |
| [history-replay.md](history-replay.md) | feature 020's export contract (requirements) and `docs/runbooks/test-server.md` | requirements |
| [template-index.schema.json](template-index.schema.json) | `specs/001-platform-foundation/contracts/template-index.schema.json` | new |
| [cli-output.schema.json](cli-output.schema.json) | `apps/cli/schemas/output.schema.json` and `docs/reference/cli.md` | new |
| [vscode-extension.md](vscode-extension.md) | `frontend/apps/vscode-extension/README.md` | new |

`events.asyncapi.yaml` grows only: `OperationalAlertWebhook.kind` gains `GIT_SYNC_STUCK`, `GIT_SYNC_PAUSED` and
`GIT_SYNC_CONFLICTS` (an enumeration gains values, `docs/event-schema-evolution.md` rule 3; the baseline is regenerated
on purpose), and the alert kind enum of the namespace alerts operation in `openapi.yaml` gains the same values.

Unchanged on purpose: `worker.proto` (interceptors are SDK-local; no protocol minor), `workflow.schema.json` (no IR
change). The synchronizer publishes no broker message of its own.
