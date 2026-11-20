# scripts

## Overview

Generated API drift guard comparing isolated regeneration with the checked-in TypeScript declarations.

## Interface

- check-api.mjs implements the Studio check:api command. [frontend/apps/studio/scripts/check-api.mjs:dir] [frontend/apps/studio/package.json:scripts.check:api]

## Invariants

- Success requires byte-for-byte equality between regenerated output and src/api/schema.d.ts. A mismatch reports npm run api and calls process.exit(1), which bypasses the finally cleanup; ordinary completion and thrown errors reach that cleanup. [frontend/apps/studio/scripts/check-api.mjs:readFileSync] [frontend/apps/studio/scripts/check-api.mjs:finally]

## Side Effects

- Creates a temporary directory, executes npx openapi-typescript, reads both files, and prints status. The finally block removes temporary output except when the explicit mismatch exit terminates the process first. [frontend/apps/studio/scripts/check-api.mjs:dir] [frontend/apps/studio/scripts/check-api.mjs:execFileSync] [frontend/apps/studio/scripts/check-api.mjs:finally]

## Verification

From the repository root, after installing the locked frontend dependencies with `npm --prefix frontend ci`:

```sh
npm --prefix frontend run lint
npm --prefix frontend run typecheck
npm --prefix frontend test
npm --prefix frontend run check:api
```

The scripts delegate to the `@sagat/studio` workspace. [frontend/package.json:scripts] [frontend/apps/studio/package.json:scripts]
