# secrets

## Overview

Namespace secret-reference administration UI for OpenBao creation, rotation, and legacy migration.

## Interface

- `Secret` aliases generated OpenAPI metadata; `secretsApi` exposes list, put/create-or-rotate, and migration requests. [frontend/apps/studio/src/features/secrets/api.ts:Secret] [frontend/apps/studio/src/features/secrets/api.ts:secretsApi]
- `SecretsPage`/`SecretList` present metadata and management actions; `SecretForm` collects new values or migrates a legacy reference. [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretsPage] [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretList] [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]

## Invariants

- Names start with a lowercase letter and contain lowercase letters, digits, underscores, dots, or dashes, up to 64 characters. New content is required and limited to 16,384 UTF-8 bytes; PostgreSQL values require username and password. [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]
- Put requests carry expectedVersion, requestId, and requiredProvider=openbao; migration carries the current expectedVersion/requestId. A successful form response must report provider openbao. [frontend/apps/studio/src/features/secrets/api.ts:secretsApi] [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]
- Unchanged retries reuse the request ID; edits clear it. Async results update the UI only while mounted and under the same captured session token. [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]
- Viewing metadata requires secret:read; create, rotate, and migrate controls require secret:manage. Stored values are never loaded back into the form or rendered in the list; value/password fields are cleared on success. [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretsPage] [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretList] [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]

## Side Effects

- Performs namespace-scoped GET secrets, PUT one secret, and POST secret migration requests. [frontend/apps/studio/src/features/secrets/api.ts:secretsApi]
- Fetches namespaces/metadata, generates request UUIDs, invalidates secret queries after writes, announces success, clears credential inputs, and closes the dialog. [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretsPage] [frontend/apps/studio/src/features/secrets/SecretsPage.tsx:SecretList] [frontend/apps/studio/src/features/secrets/SecretForm.tsx:SecretForm]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
