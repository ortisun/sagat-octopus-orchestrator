# scripts

## Overview

Shared-contract maintenance and Maven checksum experiment. [sync-shared.sh:cp] [verify-strict-checksums.sh:resolve]

## Interface

- `sync-shared.sh` copies canonical worker.proto and error-catalog.json. `verify-strict-checksums.sh` supports M2_REPO and KEEP. [sync-shared.sh:cp] [verify-strict-checksums.sh:M2_REPO] [verify-strict-checksums.sh:KEEP]

## Invariants

- Both scripts use set -euo pipefail; the checksum experiment requires an existing cached artifact and the strict-checksums configuration. [sync-shared.sh:set -euo pipefail] [verify-strict-checksums.sh:set -euo pipefail] [verify-strict-checksums.sh:strict-checksums]

## Side Effects

- Sync creates directories and overwrites copied contracts. The experiment creates scratch repositories, copies local Maven cache content, invokes Maven and removes its own scratch directory unless KEEP is set. [sync-shared.sh:mkdir] [sync-shared.sh:cp] [verify-strict-checksums.sh:mktemp] [verify-strict-checksums.sh:resolve] [verify-strict-checksums.sh:trap]

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
