# features

## Overview

Studio feature areas composed by the application routes.

## Interface

- [connections](connections/README.md) — Namespace PostgreSQL connection lifecycle, secret-backed credentials, revision-aware deletion, and managed probes.
- [database](database/README.md) — Ad hoc database execution authoring, retained-result viewing, bounded decoding, and immutable pending commands.
- [integrations](integrations/README.md) — Namespace broker connector administration and egress-policy editing.
- [operations](operations/README.md) — Namespace alert views and persistent worker-identity drain/resume controls.
- [runs](runs/README.md) — Run list/detail presentation, live graph and journal following, operator decisions, and authorized payload views.
- [scripts](scripts/README.md) — Script listing and editing, revisioned draft saving, sandbox test runs, publication/build following, and lazy Monaco editing.
- [secrets](secrets/README.md) — Namespace secret-reference administration UI for OpenBao creation, rotation, and legacy migration.
- [workflows](workflows/README.md) — Workflow listing and authoring UI with draft autosave, canvas/list views, validation, version comparison, and publication.

## Invariants

- `connections`: Connection names have 3–63 lowercase letters/digits/hyphens without consecutive hyphens; host, port, database, credentials, and execution limits are validated. TLS verify-full is normal; plaintext is offered only in development namespaces. [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm] [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:LIMIT_FIELDS]
- `database`: Ad hoc execution requires database:adhoc, database:use, and run:start, plus database:write for writes. Input must be a JSON object, timeout 1–3,600,000 ms, and the at-most-64-node document must contain a valid database node. [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:DatabaseExecutionsPage] [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:ExecutionEditor]
- `integrations`: Connector slugs have 3–63 lowercase letters/digits/hyphens without consecutive hyphens; targets, required fields, credentials, and port ranges are validated before submission. [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm]
- `operations`: Viewing requires ops:read and resolves the namespace slug to its ID; controls require worker:manage. Drain/resume reasons are trimmed to 8–2,000 characters. [frontend/apps/studio/src/features/operations/OperationsPage.tsx:OperationsPage] [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:reasonProblem]
- `runs`: URL filters accept enumerated run statuses and UUID-shaped workflow IDs; pasted run IDs normalize only when UUID-shaped. Start/signal payloads must be JSON objects within 256 KiB; unchanged request fingerprints retain their retry key. [frontend/apps/studio/src/features/runs/RunsPage.tsx:readFilters] [frontend/apps/studio/src/features/runs/RunsPage.tsx:parseRunId] [frontend/apps/studio/src/features/runs/StartRun.tsx:parseInput] [frontend/apps/studio/src/features/runs/StartRun.tsx:StartForm] [frontend/apps/studio/src/features/runs/SendSignal.tsx:SignalForm]
- `scripts`: Query keys include namespace IDs so cached script data is scoped to its namespace. [frontend/apps/studio/src/features/scripts/queries.ts:keys]
- `secrets`: Names start with a lowercase letter and contain lowercase letters, digits, underscores, dots, or dashes, up to 64 characters. New content is required and limited to 16,384 UTF-8 bytes; PostgreSQL values require username and password. [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]
- `workflows`: Workflow names start with a lowercase letter, contain lowercase letters/digits/dashes, and have 3–64 characters. [frontend/apps/studio/src/features/workflows/WorkflowsPage.tsx:WORKFLOW_NAME]

## Side Effects

- `connections`: May create an OpenBao secret before PUTing the connection; invalidates secret/connection queries, announces success, and closes the form. [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm]
- `database`: Posts ad hoc definitions to namespace database-executions and warns before unload while acknowledgement is ambiguous. [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:ExecutionEditor]
- `integrations`: Reads/writes egress policy and connectors, and can store credentials as namespace secrets before saving a connector. Saves invalidate queries, announce success, and close the form. [frontend/apps/studio/src/features/integrations/api.ts:integrationsApi] [frontend/apps/studio/src/features/integrations/IntegrationForm.tsx:IntegrationForm]
- `operations`: Reads alerts/workers and posts idempotent drain/undrain commands; workers poll every 15 seconds and alerts page through results. [frontend/apps/studio/src/features/operations/api.ts:operationsApi] [frontend/apps/studio/src/features/operations/WorkersPanel.tsx:WorkersPanel] [frontend/apps/studio/src/features/operations/AlertsPanel.tsx:AlertsPanel]
- `runs`: Run lists fetch pages, install visibility listeners, schedule refresh timers, publish snapshots, and announce loads. [frontend/apps/studio/src/features/runs/RunsPage.tsx:RunList] [frontend/apps/studio/src/features/runs/runList.ts:RunListStore]
- `scripts`: Performs HTTP reads/mutations on namespace scripts, versions, and test runs through encoded path construction. [frontend/apps/studio/src/features/scripts/api.ts:scriptsApi]
- `secrets`: Performs namespace-scoped GET secrets, PUT one secret, and POST secret migration requests. [frontend/apps/studio/src/features/secrets/api.ts:secretsApi]
- `workflows`: Issues namespace/workflow, draft, validation, version, diff, and publish requests; invalidates workflow/version caches and announces edit/save/publication outcomes. [frontend/apps/studio/src/features/workflows/WorkflowsPage.tsx:WorkflowsPage] [frontend/apps/studio/src/features/workflows/WorkflowEditorPage.tsx:WorkflowEditorPage] [frontend/apps/studio/src/features/workflows/WorkflowEditor.tsx:WorkflowEditor] [frontend/apps/studio/src/features/workflows/draftSession.ts:DraftSession.#attemptSave] [frontend/apps/studio/src/features/workflows/useValidation.ts:useValidation] [frontend/apps/studio/src/features/workflows/VersionsPanel.tsx:VersionsPanel] [frontend/apps/studio/src/features/workflows/PublishDialog.tsx:PublishBody]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
