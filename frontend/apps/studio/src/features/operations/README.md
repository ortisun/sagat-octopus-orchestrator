# operations

## Overview

Namespace alert views and persistent worker-identity drain/resume controls.

## Interface

- `OperationsPage`, `AlertsPanel`, `WorkersPanel`, and `WorkerActionDialog` expose operational views and commands. operationsApi/OperationsApi and generated Alert/Worker/Drain aliases form the API boundary. [frontend/apps/studio/src/features/operations/OperationsPage.tsx:OperationsPage] [frontend/apps/studio/src/features/operations/AlertsPanel.tsx:AlertsPanel] [frontend/apps/studio/src/features/operations/WorkersPanel.tsx:WorkersPanel] [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:WorkerActionDialog] [frontend/apps/studio/src/features/operations/api.ts:operationsApi] [frontend/apps/studio/src/features/operations/api.ts:OperationsApi] [frontend/apps/studio/src/features/operations/api.ts:Alert] [frontend/apps/studio/src/features/operations/api.ts:Worker] [frontend/apps/studio/src/features/operations/api.ts:Drain]
- `FlowControlKeysPanel` lists the namespace's live concurrency keys (`GET /namespaces/{namespaceId}/concurrency-keys`) and rate-limit keys (`GET /namespaces/{namespaceId}/rate-limit-keys`) of feature 022, oldest wait first, each as a table with a group filter and "Load more" paging; `flowControlKeysApi` is its API boundary and `keyLabelWords`, `scopeWords` and `poolWords` its words. [frontend/apps/studio/src/features/operations/FlowControlKeysPanel.tsx:FlowControlKeysPanel] [frontend/apps/studio/src/features/operations/flowControlKeys.ts:flowControlKeysApi] [frontend/apps/studio/src/features/operations/flowControlKeys.ts:keyLabelWords]
- `WorkerIdentity`, groupWorkerIdentities, alertKind, and safeRunbookHref model sessions/drains and safe navigation. [frontend/apps/studio/src/features/operations/model.ts:WorkerIdentity] [frontend/apps/studio/src/features/operations/model.ts:groupWorkerIdentities] [frontend/apps/studio/src/features/operations/model.ts:alertKind] [frontend/apps/studio/src/features/operations/model.ts:safeRunbookHref]

## Invariants

- Viewing requires ops:read and resolves the namespace slug to its ID; controls require worker:manage. Drain/resume reasons are trimmed to 8–2,000 characters. [frontend/apps/studio/src/features/operations/OperationsPage.tsx:OperationsPage] [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:reasonProblem]
- The key listings follow the platform's order and cursor: the cursor is opaque and only sent back, the group filter is sent as typed and a `422` is shown as the platform's answer. A key is shown by its label only; a `null` label is said hidden, and the instants of such a key are said to be cut to the minute, as the platform cuts them. Each listing is a table with a caption, column headers and the group as the header of its row, inside a region the keyboard can scroll (decision D022-150 item 7). A platform that answers a listing's route with a `404` (it has no such listing; the namespace itself was resolved by the page) shows that listing as unavailable, not as a failure (FR-007-010); a scope or a pool this Studio does not know is shown as unsupported. A group row whose sums were read across pages (`spansPages`) says under its name, inside the row header that assistive technology reads with every count, that its totals were read across pages and may be above the limit (decision D022-111). [frontend/apps/studio/src/features/operations/FlowControlKeysPanel.tsx:FlowControlKeysPanel] [frontend/apps/studio/src/features/operations/flowControlKeys.ts:spansPagesWords]
- Sessions and persistent drains group/sort by subject, retaining drains with no current session. Runbook links allow same-origin root-relative references or HTTPS. [frontend/apps/studio/src/features/operations/model.ts:groupWorkerIdentities] [frontend/apps/studio/src/features/operations/model.ts:safeRunbookHref]

## Side Effects

- Reads alerts/workers and posts idempotent drain/undrain commands; workers poll every 15 seconds and alerts page through results. [frontend/apps/studio/src/features/operations/api.ts:operationsApi] [frontend/apps/studio/src/features/operations/WorkersPanel.tsx:WorkersPanel] [frontend/apps/studio/src/features/operations/AlertsPanel.tsx:AlertsPanel]
- The key listings read pages on demand (first page, then one per "Load more" or filter change) and never poll. [frontend/apps/studio/src/features/operations/FlowControlKeysPanel.tsx:KeyListing]
- Retains request keys for unchanged action payloads, aborts obsolete/unmounted requests, refreshes data, and announces results. [frontend/apps/studio/src/features/operations/WorkerActionDialog.tsx:WorkerActionForm]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
