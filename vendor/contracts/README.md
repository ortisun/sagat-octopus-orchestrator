# Vendor contracts

## Overview

Architectural documentation for third-party source snapshots lives here so the verified upstream trees remain unchanged.

## Interface

- [rskafka](rskafka/README.md) documents the source under `vendor/rskafka`.

## Invariants

- The snapshot checker verifies the exact upstream file set and file hashes, allowing only the reviewed `zstd` dependency version replacement. Documentation additions belong outside that snapshot. [scripts/check-vendor.py:verify]

## Side Effects

- None: these contracts are documentation. Running the verification command reads the snapshot and provenance manifest and reports success or failure. [scripts/check-vendor.py:verify] [scripts/check-vendor.py:main]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```
