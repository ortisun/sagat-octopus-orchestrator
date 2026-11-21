# crates/infra-schema/src/classification

## Overview

Bounded classification analysis and disclosure helpers: compiled-carrier caching, exact JSON-number checks, publication-location walking and audience-aware projection.

## Interface

- `CacheStats` is publicly re-exported. `CarrierCache`, `Outcome`, `stats` and `get_or_compile` remain parent-private. [cache.rs:CacheStats] [cache.rs:CarrierCache::get_or_compile] [../classification.rs:CacheStats]
- Number helpers expose parent-private `NumberChecks`, `Lexeme` navigation, `Lexemes::scan`, `NumericProfile::{of,checks,admits}` and `schema_requires_integer`. [numbers.rs:NumberChecks] [numbers.rs:Lexeme] [numbers.rs:Lexemes::scan] [numbers.rs:NumericProfile] [numbers.rs:schema_requires_integer]
- `analyze_value`, `visit_classification_locations` and `resolve_classification_location` are publicly re-exported. Byte-oriented `analyze` stays inside the private publication module. These consume application carrier/location types and `ClassificationLocationIndex`. [publication.rs:analyze] [publication.rs:analyze_value] [publication.rs:visit_classification_locations] [publication.rs:resolve_classification_location] [../classification.rs:analyze_value]
- Projection uses parent-private `Walk::{new,steps,charge,expand}`, `precheck`, `ProjectionBudget` and `Projector::{new,run}`. [walk.rs:Walk] [walk.rs:precheck] [walk.rs:ProjectionBudget] [walk.rs:Projector::run]

## Invariants

- The cache allows four concurrent compiles, single-flights each key, bounds entries/weight/remembered failures, and converts an abandoned or panicking leader into `SchemaUnusable`. [cache.rs:MAX_CONCURRENT_COMPILES] [cache.rs:CarrierCache::get_or_compile] [cache.rs:Finish::drop]
- Numeric validation retains lexical spellings, checks exact representability and safe comparisons around the `2^53` bound, and rejects fractions that would round into integers. [numbers.rs:SAFE_BOUND] [numbers.rs:NumericProfile::admits] [numbers.rs:safe_comparison] [numbers.rs:is_rounded_fraction]
- Publication rejects oversized/deep carriers, external or unresolved references, reference cycles/expansion overflow and classification annotations under unsupported keywords. [publication.rs:process_carrier] [publication.rs:CarrierWalk::expand] [publication.rs:CarrierWalk::scan_schema]
- Effective classification can only become stricter down a location; loosening is reported and undeclared open children produce `InheritedOpen`. [publication.rs:CarrierWalk::walk_location] [publication.rs:CarrierWalk::warn_if_open]
- Projection caps walk steps/deadline, reference expansion, depth/nodes, container children, response bytes and string disclosure; missing or unsupported child governance is floored to at least PII. [walk.rs:Walk::charge] [walk.rs:Projector::child_plan] [walk.rs:ProjectionBudget::admit_node] [walk.rs:Projector::object]

## Side Effects

- Cache access mutates mutex/condvar state and atomic counters, may block followers/permit waiters, and increments `classification_cache_total` metrics. [cache.rs:CarrierCache::get_or_compile] [cache.rs:CarrierCache::record_hit] [cache.rs:CarrierCache::record_miss]
- Publication builds in-memory findings, reports and indexes and invokes visitor callbacks. [publication.rs:analyze_document] [publication.rs:Analysis::finding] [publication.rs:CarrierWalk::emit_location]
- Projection changes its in-memory walk/budget and checks an optional monotonic deadline; disclosed values can be truncated or omitted. [walk.rs:Walk::charge] [walk.rs:ProjectionBudget] [walk.rs:Projector::node]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-schema --locked
```

These commands were not run in this documentation-only pass.
