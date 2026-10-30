# crates/infra-ir

## Overview

sagat-infra-ir converts JSON workflow IR into validated domain models, canonical forms and digests.

## Interface

- JsonIrCodec, JsonEvaluator, declared-operation evaluators, payload projection, database-result validation and CronCalendar expose the IR boundary. [src/lib.rs:JsonIrCodec] [src/evaluator.rs:JsonEvaluator] [src/operation_evaluator.rs:JsonDeclaredOperationEvaluator] [src/payload_projection.rs:JsonPayloadProjector] [src/cron.rs:CronCalendar]
- [src](./src/README.md) — JSON IR boundary for schema validation, capability-gated conversion, canonical digests, classification, bounded expressions/declared operations, payload projection, database results and document diffing. [src/lib.rs:JsonIrCodec]

## Invariants

- Authoring/stored parsing, canonicalization, expression/schema limits are distinct. Published collection/loop/continuation support has explicitly checked shapes; maxInstances/maxDepth are not general nesting or workload acceptance evidence. [src/lib.rs:validated_document] [src/lib.rs:validated_stored_document] [src/convert.rs:document_limits] [src/convert.rs:published_collection_definition] [src/convert.rs:published_loop_definition] [src/continuation.rs:parse_published]

## Side Effects

- Most work is in memory; cron validation reads current UTC time. [src/lib.rs:JsonIrCodec] [src/cron.rs:validate]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-ir --locked
```

These commands were not run in this documentation-only pass.
