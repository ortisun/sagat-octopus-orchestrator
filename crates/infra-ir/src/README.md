# crates/infra-ir/src

## Overview

JSON IR boundary for schema validation, capability-gated conversion, canonical digests, classification, bounded expressions/declared operations, payload projection, database results and document diffing.

## Interface

- JsonIrCodec implements classification analysis, draft checks, diff/rules, authoring/stored parsing, declared-state publication/initialization and narrower structured/state/continuation parsers. [lib.rs:JsonIrCodec] [lib.rs:IrCodec] [lib.rs:parse_structured] [lib.rs:parse_continuation]
- JsonEvaluator implements condition, correlation/idempotency, child/continuation/database/task/collection/loop/event/reconciliation evaluation. Converter kinds cover the implemented node vocabulary while rejecting legacy placeholders. [evaluator.rs:JsonEvaluator] [evaluator.rs:ExpressionEvaluator] [convert.rs:kind_of] [convert.rs:unsupported]
- Declared schemas/query/update/state/parsed definitions and limits expose authored state contracts; JsonDeclaredOperationEvaluator, EvaluatedOperationValue, QueryEvaluation and UpdateEvaluationPlan bind decisions to observed state/run revisions. [declared_state.rs:DeclaredSchema] [declared_state.rs:DeclaredQueryDefinition] [declared_state.rs:DeclaredUpdateDefinition] [declared_state.rs:DeclaredStateDefinition] [declared_state.rs:ParsedDeclaredState] [operation_evaluator.rs:JsonDeclaredOperationEvaluator] [operation_evaluator.rs:EvaluatedOperationValue] [operation_evaluator.rs:QueryEvaluation] [operation_evaluator.rs:UpdateEvaluationPlan]
- ChainPolicyDefinition, ContinueAsNewDefinition and ParsedContinuation describe continuation parsing. JsonDatabaseResultValidator validates retained result envelopes; JsonPayloadProjector produces value-free structures; CronCalendar implements five-field/IANA-zone scheduling. [continuation.rs:ChainPolicyDefinition] [continuation.rs:ContinueAsNewDefinition] [continuation.rs:ParsedContinuation] [database_result.rs:JsonDatabaseResultValidator] [payload_projection.rs:JsonPayloadProjector] [cron.rs:CronCalendar]
- Classification checks/location walks, carrier-schema checks, canonical JSON/digests, schema diagnostics, database-node validation and document diff are internal codec helpers. [classification_check.rs:check] [classification_check.rs:visit_locations] [carrier_schema_check.rs:check] [canonical.rs:canonical_json] [canonical.rs:digest_of] [schema.rs:issues] [database.rs:operation] [diff.rs:diff]

## Invariants

- Authoring enforces the 1 MiB IR limit, classification, embedded schema and carrier-schema checks; stored parsing deliberately skips later authoring classification rules while retaining structural/executable checks. Canonical JSON sorts object keys, preserves array order and hashes SHA-256. [lib.rs:read_document] [lib.rs:validated_document] [lib.rs:validated_stored_document] [canonical.rs:canonical_json] [canonical.rs:digest_of]
- Executable expressions compile before publication; structured/switch expressions are capped at 1024 bytes and trigger variables at event/data. Document maxInstances/maxDepth are bounded 1..=100000/1..=8 authoring values, not workload admission evidence. [convert.rs:kind_of] [lib.rs:check_expressions] [convert.rs:document_limits]
- Published collections/loops reject chain policy/triggers and admit only the explicitly checked root/body shapes; body chains are limited to eight executable nodes and a terminal return. General nesting and approval bodies remain separately gated. [convert.rs:published_collection_definition] [convert.rs:published_loop_definition] [convert.rs:is_collection_body] [convert.rs:is_activity_chain] [convert.rs:approval_in_a_body]
- Continuation publication requires bounded chain policy and only direct/conditional/wait/approval-gated shapes checked by the parser; event-wait entry adds restrictions on timeout routes/lookback/state. [continuation.rs:policy] [continuation.rs:parse_published]
- Declared schemas are canonical, at most 64 KiB/depth 16, forbid external/dynamic references and validate bounds; queries/updates cap at 32 each. Evaluated values are schema-checked, content-addressed and at most 256 KiB. [declared_state.rs:schema] [declared_state.rs:MAX_DECLARED_OPERATIONS] [declared_state.rs:compiled_expression] [operation_evaluator.rs:evaluated_value]
- Diff bounds changes/findings/names/path depth; schema diagnostics cap issues/message sizes. Payload projection bounds input/depth/nodes/children and omits keys/scalars; database results reject unknown fields/truncation, excess size/rows and inconsistent column/cell shapes. [diff.rs:MAX_CHANGES] [diff.rs:MAX_FINDINGS] [diff.rs:MAX_NAME] [diff.rs:MAX_SEGMENTS] [schema.rs:MAX_ISSUES] [schema.rs:MAX_MESSAGE] [payload_projection.rs:JsonPayloadProjector] [database_result.rs:validate]

## Side Effects

- Parsing/conversion/diff/classification/expression work returns in-memory values/errors without persistence or network I/O. Projection returns structural kinds/counts without input keys/scalars. [lib.rs:JsonIrCodec] [operation_evaluator.rs:JsonDeclaredOperationEvaluator] [payload_projection.rs:project]
- CronCalendar validation reads current UTC time to check that a next occurrence exists. [cron.rs:validate]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-ir --locked
```

These commands were not run in this documentation-only pass.
