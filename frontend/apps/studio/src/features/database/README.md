# database

## Overview

Ad hoc database execution authoring, retained-result viewing, bounded decoding, and immutable pending commands.

## Interface

- `DatabaseExecutionsPage` and `DatabaseResultViewer` expose execution/result UI; DatabaseResult/MAX_RESULT_BYTES/readDatabaseResult decode results, while DatabaseIntent/useDatabaseCommand retain command identity. [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:DatabaseExecutionsPage] [frontend/apps/studio/src/features/database/DatabaseResultViewer.tsx:DatabaseResultViewer] [frontend/apps/studio/src/features/database/readResult.ts:DatabaseResult] [frontend/apps/studio/src/features/database/readResult.ts:MAX_RESULT_BYTES] [frontend/apps/studio/src/features/database/readResult.ts:readDatabaseResult] [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:DatabaseIntent] [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:useDatabaseCommand]

## Invariants

- Ad hoc execution requires database:adhoc, database:use, and run:start, plus database:write for writes. Input must be a JSON object, timeout 1–3,600,000 ms, and the at-most-64-node document must contain a valid database node. [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:DatabaseExecutionsPage] [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:ExecutionEditor]
- Result reads combine caller cancellation with 30 seconds, limit success to 4 MiB/errors to 64 KiB, require application/json, at most 10,000 rows, exact column widths, known cell types, and complete non-truncated results. [frontend/apps/studio/src/features/database/readResult.ts:readDatabaseResult] [frontend/apps/studio/src/features/database/readResult.ts:MAX_RESULT_BYTES]
- One immutable intent per session/scope survives ambiguous submission. Retention is bounded to eight pending intents of at most 2 MiB; retry resends the same intent, and tenant/subject change disposes retained commands. [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:Command.send] [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:commandFor] [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:useDatabaseCommand]

## Side Effects

- Posts ad hoc definitions to namespace database-executions and warns before unload while acknowledgement is ambiguous. [frontend/apps/studio/src/features/database/DatabaseExecutionsPage.tsx:ExecutionEditor]
- Fetches retained results on request, aborts prior/closed reads, clears displayed content on close, and signals unauthenticated sessions on HTTP 401. [frontend/apps/studio/src/features/database/DatabaseResultViewer.tsx:DatabaseResultViewer] [frontend/apps/studio/src/features/database/readResult.ts:readDatabaseResult]
- Creates request UUIDs, retains cloned bodies in session memory, subscribes to identity changes, and aborts commands on disposal. [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:Command.send] [frontend/apps/studio/src/features/database/useDatabaseCommand.ts:commandFor]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
