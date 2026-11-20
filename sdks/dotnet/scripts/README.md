# scripts

## Overview

Synchronization of copied protocol and embedded error catalog. [sync-shared.sh:cp]

## Interface

- sync-shared.sh is the maintenance entry point. [sync-shared.sh:#!/usr/bin/env bash]

## Invariants

- Uses set -euo pipefail and derives repository/SDK roots from the script location. [sync-shared.sh:set -euo pipefail]

## Side Effects

- Creates proto if needed and overwrites worker.proto and error-catalog.json with repository copies. [sync-shared.sh:mkdir] [sync-shared.sh:cp]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
