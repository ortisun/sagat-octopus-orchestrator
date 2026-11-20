# crates/infra-schema/src

## Overview

Bounded JSON Schema checking/inference, signed artifact-bundle validation and classification projection.

## Interface

- `Schemas` implements input/schema checking; public `validate` returns at most three path/rule-only violations and `infer` derives a sample schema. [lib.rs:Schemas] [lib.rs:validate] [lib.rs:infer]
- Bundle limits, `BundleFile`, `files`, `decoded_files`, `target` and `Bundles` expose checked text/binary artifacts and BundleChecker validation. [bundle.rs:MAX_FILES] [bundle.rs:MAX_BUNDLE_BYTES] [bundle.rs:MAX_DECODED_BYTES] [bundle.rs:MAX_EXPANDED_BYTES] [bundle.rs:BundleFile] [bundle.rs:files] [bundle.rs:decoded_files] [bundle.rs:target] [bundle.rs:Bundles]
- `Classifier` exposes cache stats/counted projection and implements analyze/prepare/project. `WalkCounters`, `CacheStats` and publication analysis/location helpers are exposed through classification. [classification.rs:Classifier] [classification.rs:WalkCounters] [classification/cache.rs:CacheStats] [classification/publication.rs:analyze_value] [classification/publication.rs:visit_classification_locations] [classification/publication.rs:resolve_classification_location]
- [classification](./classification/README.md) — Bounded classification analysis and disclosure helpers: compiled-carrier caching, exact JSON-number checks, publication-location walking and audience-aware projection. [classification/cache.rs:CacheStats]

## Invariants

- Schemas must be bounded JSON objects, cannot resolve external references and use regex/DFA limits of 1 MiB. [lib.rs:check] [lib.rs:PATTERN_SIZE_LIMIT] [lib.rs:compile]
- Bundles cap canonical JSON at 2 MiB, 32 plain-named files, decoded binary at 1.4 MiB and expanded Go ELF at 8 MiB; duplicate members/noncanonical encodings are rejected. Accepted formats are narrow language/version/ABI/target combinations. [bundle.rs:MAX_BUNDLE_BYTES] [bundle.rs:MAX_FILES] [bundle.rs:MAX_DECODED_BYTES] [bundle.rs:MAX_EXPANDED_BYTES] [bundle.rs:unique_json] [bundle.rs:decoded_files]
- Carrier compilation bounds bytes/depth/patterns/references; projection rejects legacy/oversized/invalid/schema-mismatched or numerically unsafe payloads and charges deadline/step work. [classification.rs:compile_carrier] [classification.rs:Classifier::run]

## Side Effects

- Schema/bundle checks use CPU/memory without resolving external files/network references. Classification uses a process-local synchronized cache/metrics and in-memory visitor/projection work. [lib.rs:compile] [bundle.rs:decoded_files] [classification/cache.rs:CarrierCache::get_or_compile] [classification/publication.rs:visit_classification_locations] [classification/walk.rs:Projector::run]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-schema --locked
```

These commands were not run in this documentation-only pass.
