# scripts

## Overview

Repository synchronization for the embedded protocol and error catalog. [sync-shared.mjs:writeFileSync]

## Interface

- `npm run sync` invokes sync-shared.mjs. [../package.json:sync]

## Invariants

- The canonical inputs are the platform worker.proto and shared SDK error-catalog.json; generated catalog.ts includes a generated-file notice. [sync-shared.mjs:readFileSync]

## Side Effects

- Creates src if needed and overwrites proto/worker.proto and src/catalog.ts. [sync-shared.mjs:mkdirSync] [sync-shared.mjs:writeFileSync]

## Verification

Run from the repository root:

```sh
cd sdks/typescript && npm test
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
