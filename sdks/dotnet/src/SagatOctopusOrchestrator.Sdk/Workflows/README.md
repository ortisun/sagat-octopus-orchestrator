# Workflows

## Overview

Fluent workflow, nested-flow and body authoring with local state/database validation. [Flow.cs:WorkflowBuilder] [StateDefinitionValidator.cs:StateDefinitionValidator]

## Interface

- `FlowBase<TSelf>`, Flow, Body, WorkflowBuilder and Workflow construct documents; Database extends flow authoring and SwitchCase models branching. [Flow.cs:FlowBase] [Flow.cs:WorkflowBuilder] [Flow.cs:Workflow] [FlowDatabase.cs:Database] [SwitchCase.cs:SwitchCase]
- Database contracts include ConnectionRef, DatabaseOperation, DatabaseType, DatabaseParameter and DatabaseOptions. [DatabaseOptions.cs:ConnectionRef] [DatabaseOptions.cs:DatabaseOptions]
- WorkflowOptions supplies retry/step/script/event/child/workflow/parallel/saga/collection/loop/approval/chain options, state/query/update/disclosure definitions and trigger kinds/values. StateDefinitionValidator.Build is the internal state-validation boundary. [WorkflowOptions.cs:StepOptions] [WorkflowOptions.cs:ScriptOptions] [WorkflowOptions.cs:CollectionOptions] [WorkflowOptions.cs:ApprovalOptions] [WorkflowOptions.cs:ChainPolicyOptions] [WorkflowOptions.cs:StateDefinition] [WorkflowOptions.cs:Trigger] [StateDefinitionValidator.cs:Build]

## Invariants

- Build validates names, unique IDs, local routes, closed bodies, acyclic body references and top-level continuation. Durations require whole milliseconds and node/workflow bounds are explicit. [Flow.cs:Build] [Flow.cs:WholeMillis]
- State results use closed bounded schemas, at most 32 queries/updates each, bounded expressions and declared-business-result-v1 disclosure. Database authoring accepts logical connections and bounded typed parameters; local validation does not establish server acceptance or execute SQL. [StateDefinitionValidator.cs:Build] [DatabaseOptions.cs:DatabaseOptions]

## Side Effects

- Builders clone, validate and construct JSON in memory without I/O, clock reads or randomness. [Flow.cs:WorkflowBuilder]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
