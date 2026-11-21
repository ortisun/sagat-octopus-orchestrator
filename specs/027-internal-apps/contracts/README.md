# Contract fragments — feature 027 (drafts, not merged, not implemented)

| Fragment | Merge target (task) | Notes |
|---|---|---|
| [apps.openapi.yaml](apps.openapi.yaml) | `specs/001-platform-foundation/contracts/openapi.yaml` (T027-004) | 20 paths, tag `Apps`, `x-amends-existing-operations` (reserved `app:` key prefix), `x-planned: '027'`, `appSession` scheme; refs resolve after merge; generated Studio types regenerate (`npm run api`) |
| [app.schema.json](app.schema.json) | new `specs/001-platform-foundation/contracts/app.schema.json` (T027-004) | App document; JSON Schema 2020-12; shape only, the codec adds semantic checks |
| [examples/refund-desk.app.json](examples/refund-desk.app.json) | `tests/contract/internal-apps/fixtures/` (T027-005) | Illustrative; validated against `app.schema.json` on 2026-10-26 with Python `jsonschema` in a scratch environment (0 errors; five negative cases rejected) |
| [manifest-apps.schema.json](manifest-apps.schema.json) | `specs/011-gitops-versioning/contracts/manifest.schema.json` (T027-036) | Additive optional `apps`, `appComponents` |
| [component-frame.md](component-frame.md) | `docs/architecture/security-and-sandbox.md` section and `frontend` contract test (T027-046, T027-049) | Host ↔ frame protocol `sagat-app-component/1` |
| [errors.md](errors.md) | `Error.details.reason` description in the OpenAPI (T027-004) | No new `Error.code` |

Unchanged by this feature: `workflow.schema.json` (no IR change), `worker.proto` and its baseline (no
protocol minor), `events.asyncapi.yaml` and `events.baseline.json` (no message), the five SDKs'
public surface.
