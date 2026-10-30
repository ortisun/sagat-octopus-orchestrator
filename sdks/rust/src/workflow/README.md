# workflow

## Overview

Typed workflow primitives, state definitions and logical database IR construction. [primitives.rs:WaitEventOptions] [state.rs:StateDefinition] [database.rs:DatabaseOptions]

## Interface

- Primitives expose WaitEventOptions, ChildCancelPolicy/ChildOptions, SwitchCase, CollectionOptions, LoopOptions, ApprovalRole/ApprovalPolicy, ScriptOptions, WorkflowLimits, TriggerKind/Trigger and Builder methods for metadata, triggers, bounds, event/child/decision/collection/loop/body/continuation/approval/script authoring. [primitives.rs:WaitEventOptions] [primitives.rs:Trigger] [primitives.rs:impl Builder]
- State authoring exposes DisclosurePolicy, QueryDefinition, UpdateDefinition, StateDefinition and Builder::state. Database authoring exposes ConnectionRef, DatabaseOperation/Type/Value/Parameter/Options and Builder::database. [state.rs:DisclosurePolicy] [state.rs:StateDefinition] [state.rs:state] [database.rs:ConnectionRef] [database.rs:DatabaseOptions] [database.rs:database]
- The parent workflow module supplies Builder, activity/retry/saga options, base node methods, build/json and canonical_json. [../workflow.rs:Builder] [../workflow.rs:canonical_json]

## Invariants

- Builder retains the first validation error. Build checks nonempty workflows, unique IDs and resolved references, joined-flow/body cycles, return placement and continuation policy. Canonical JSON uses UTF-16 key ordering and safe-number rules. [../workflow.rs:fail] [../workflow.rs:build] [../workflow.rs:canonical_json]
- State definitions require bounded closed result schemas, disclosure, bounded expressions and at most 32 queries/updates each. Database nodes validate logical names, statements, parameters, timeouts and a 256 KiB canonical parameter budget; they only emit IR. [state.rs:state] [database.rs:database]

## Side Effects

- Builder calls mutate shared in-memory draft state; build/json return owned documents/text without network or filesystem I/O. [../workflow.rs:change] [../workflow.rs:build]

## Verification

Run from the repository root:

```sh
cd sdks/rust && cargo test --locked
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
