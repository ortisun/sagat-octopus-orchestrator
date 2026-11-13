# crates/infra-schema

## Overview

sagat-infra-schema provides bounded JSON Schema validation/inference, bundle checks and classification projection.

## Interface

- Schemas, Bundles and Classifier expose validation/classification; cache statistics, publication-location analysis and audience projection support the boundary. [src/lib.rs:Schemas] [src/bundle.rs:Bundles] [src/classification.rs:Classifier] [src/classification/cache.rs:CacheStats] [src/classification/publication.rs:analyze_value] [src/classification/walk.rs:Projector::run]
- [src](./src/README.md) — Bounded JSON Schema checking/inference, signed artifact-bundle validation and classification projection. [src/lib.rs:Schemas]

## Invariants

- Schemas/bundles/carrier compilation/projection enforce explicit byte/depth/node/reference limits and reject external references. [src/lib.rs:check] [src/bundle.rs:MAX_BUNDLE_BYTES] [src/classification.rs:compile_carrier]

## Side Effects

- Validation is local CPU/memory work; classification mutates a synchronized process cache and emits bounded metrics. [src/classification/cache.rs:CarrierCache::get_or_compile] [src/classification/cache.rs:CarrierCache::record_hit]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-schema --locked
```

These commands were not run in this documentation-only pass.
