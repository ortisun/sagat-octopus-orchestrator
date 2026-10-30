# connections

## Overview

Namespace PostgreSQL connection lifecycle, secret-backed credentials, revision-aware deletion, and managed probes.

## Interface

- `ConnectionsPage`, `ConnectionForm`, `ConnectionProbe`, and `DeleteConnection` expose lifecycle controls; connectionsApi and Connection/ConnectionWrite aliases define the API boundary. [frontend/apps/studio/src/features/connections/ConnectionsPage.tsx:ConnectionsPage] [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm] [frontend/apps/studio/src/features/connections/ConnectionProbe.tsx:ConnectionProbe] [frontend/apps/studio/src/features/connections/DeleteConnection.tsx:DeleteConnection] [frontend/apps/studio/src/features/connections/api.ts:connectionsApi] [frontend/apps/studio/src/features/connections/api.ts:Connection] [frontend/apps/studio/src/features/connections/api.ts:ConnectionWrite]

## Invariants

- Connection names have 3–63 lowercase letters/digits/hyphens without consecutive hyphens; host, port, database, credentials, and execution limits are validated. TLS verify-full is normal; plaintext is offered only in development namespaces. [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm] [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:LIMIT_FIELDS]
- New credentials require secret:manage and confirmation that OpenBao stored the value. Viewing requires namespace:read, editing/deletion database:manage, probe visibility run:read, and probe start database:use/database:adhoc/run:start. [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm] [frontend/apps/studio/src/features/connections/ConnectionsPage.tsx:ConnectionsPage] [frontend/apps/studio/src/features/connections/ConnectionsPage.tsx:ConnectionList] [frontend/apps/studio/src/features/connections/ConnectionProbe.tsx:ConnectionProbe]
- Only ACTIVE connections can start a probe, which records the tested revision; historical success is not a promise of future availability. Deletion carries expectedRevision and a stable requestId, preserving prior effects/history. [frontend/apps/studio/src/features/connections/ConnectionProbe.tsx:ConnectionProbe] [frontend/apps/studio/src/features/connections/api.ts:connectionsApi] [frontend/apps/studio/src/features/connections/DeleteConnection.tsx:DeleteConnection]

## Side Effects

- May create an OpenBao secret before PUTing the connection; invalidates secret/connection queries, announces success, and closes the form. [frontend/apps/studio/src/features/connections/ConnectionForm.tsx:ConnectionForm]
- Gets probe evidence, posts an immutable retained probe command, invalidates evidence, and polls every second while the probe run is nonterminal. [frontend/apps/studio/src/features/connections/ConnectionProbe.tsx:ConnectionProbe]
- Lists/PUTs/DELETEs namespace connections; deletion invalidates the list, announces completion, and closes the dialog. [frontend/apps/studio/src/features/connections/api.ts:connectionsApi] [frontend/apps/studio/src/features/connections/DeleteConnection.tsx:DeleteConnection]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
