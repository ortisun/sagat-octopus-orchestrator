# operations

## Overview

Namespace alert views and persistent worker-identity drain/resume controls.

## Interface

- `OperationsPage`, `AlertsPanel`, `WorkersPanel`, and `WorkerActionDialog` expose operational views and commands. operationsApi/OperationsApi and generated Alert/Worker/Drain aliases form the API boundary. [frontend/apps/studio/src/features/operations/OperationsPage.tsx:OperationsPage] [frontend/apps/studio/src/features/operations/AlertsPanel.tsx:AlertsPanel] [frontend/apps/studio/src/features/operations/WorkersPanel.tsx:WorkersPanel] [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:WorkerActionDialog] [frontend/apps/studio/src/features/operations/api.ts:operationsApi] [frontend/apps/studio/src/features/operations/api.ts:OperationsApi] [frontend/apps/studio/src/features/operations/api.ts:Alert] [frontend/apps/studio/src/features/operations/api.ts:Worker] [frontend/apps/studio/src/features/operations/api.ts:Drain]
- `WorkerIdentity`, groupWorkerIdentities, alertKind, and safeRunbookHref model sessions/drains and safe navigation. [frontend/apps/studio/src/features/operations/model.ts:WorkerIdentity] [frontend/apps/studio/src/features/operations/model.ts:groupWorkerIdentities] [frontend/apps/studio/src/features/operations/model.ts:alertKind] [frontend/apps/studio/src/features/operations/model.ts:safeRunbookHref]

## Invariants

- Viewing requires ops:read and resolves the namespace slug to its ID; controls require worker:manage. Drain/resume reasons are trimmed to 8–2,000 characters. [frontend/apps/studio/src/features/operations/OperationsPage.tsx:OperationsPage] [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:reasonProblem]
- Sessions and persistent drains group/sort by subject, retaining drains with no current session. Runbook links allow same-origin root-relative references or HTTPS. [frontend/apps/studio/src/features/operations/model.ts:groupWorkerIdentities] [frontend/apps/studio/src/features/operations/model.ts:safeRunbookHref]

## Side Effects

- Reads alerts/workers and posts idempotent drain/undrain commands; workers poll every 15 seconds and alerts page through results. [frontend/apps/studio/src/features/operations/api.ts:operationsApi] [frontend/apps/studio/src/features/operations/WorkersPanel.tsx:WorkersPanel] [frontend/apps/studio/src/features/operations/AlertsPanel.tsx:AlertsPanel]
- Retains request keys for unchanged action payloads, aborts obsolete/unmounted requests, refreshes data, and announces results. [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:WorkerActionForm]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
