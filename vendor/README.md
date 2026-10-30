# Vendor snapshots

## Overview

This directory holds the reviewed `rskafka` 0.6.0 source snapshot, its provenance manifest, and separate architectural documentation. The upstream README, examples, attribution, and license files remain in the source snapshot. [vendor/rskafka.provenance.json:package] [vendor/rskafka.provenance.json:version] [vendor/rskafka.provenance.json:registryArchiveSHA256]

## Interface

- [Contract index](contracts/README.md) leads to the [rskafka architecture contract](contracts/rskafka/README.md) and its [public client boundary](contracts/rskafka/src/client/README.md).
- [Upstream rskafka README](rskafka/README.md) retains the package's usage and development notes.
- [Provenance manifest](rskafka.provenance.json) records the registry archive identity and upstream file hashes.
- The snapshot retains its [MIT](rskafka/LICENSE-MIT) and [Apache-2.0](rskafka/LICENSE-APACHE) licenses.

## Invariants

- `check-vendor.py` requires the reviewed archive identity, exact 84-file manifest, and matching file bytes. It permits only the normalized Cargo manifest's reviewed `zstd` version replacement from 0.13 to 0.14.3. [scripts/check-vendor.py:verify] [scripts/check-vendor.py:FILE_COUNT] [scripts/check-vendor.py:OLD] [scripts/check-vendor.py:NEW]
- Architectural contracts stay under `vendor/contracts`; adding README files inside `vendor/rskafka` would violate the snapshot's exact-file-set check. [scripts/check-vendor.py:verify]

## Side Effects

- Verification reads snapshot bytes and provenance, hashes them, and prints its result. [scripts/check-vendor.py:verify] [scripts/check-vendor.py:main]
- The library client performs Kafka metadata, topic, and record operations through broker connections; these documentation files add no runtime effects. [vendor/rskafka/src/client/mod.rs:ClientBuilder::build] [vendor/rskafka/src/client/controller.rs:ControllerClient] [vendor/rskafka/src/client/partition.rs:PartitionClient]

## Verification

From the repository root:

```sh
python3 scripts/check-vendor.py
```
