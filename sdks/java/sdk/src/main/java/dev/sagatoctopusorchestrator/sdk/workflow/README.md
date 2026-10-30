# workflow

## Overview

Workflow IR authoring and local validation. [WorkflowBuilder.java:WorkflowBuilder]

## Interface

- `WorkflowBuilder` provides named workflow construction, metadata, triggers, schemas, limits, state, chain policy, named bodies, appended flows, `build` and `toJson`. [WorkflowBuilder.java:WorkflowBuilder]
- `Flow` provides `create`, `body`, `database`, `step`, `script`, `sleep`/`sleepMs`, `waitForEvent`, `invokeChild`, `condition`, `switchCase`, `forEach`/`map`, `whileLoop`/`repeatUntil`, `approval`, `continueAsNew`, `parallel`, `saga`, `end`, `returnValue`, and `Join.ALL`/`ANY`. [Flow.java:Flow]
- `Trigger` provides trigger kinds, factories, field setters, `with`, and `fromJson`. [Trigger.java:Trigger]
- Option builders: `ChildOptions`, `DatabaseOptions` (connection, operation, type and parameter contracts), `RetryOptions`, `SagaOptions`, `ScriptOptions`, `StepOptions`, `WaitForEventOptions`. [ChildOptions.java:ChildOptions] [DatabaseOptions.java:DatabaseOptions] [RetryOptions.java:RetryOptions] [SagaOptions.java:SagaOptions] [ScriptOptions.java:ScriptOptions] [StepOptions.java:StepOptions] [WaitForEventOptions.java:WaitForEventOptions]
- Value contracts: `ApprovalOptions`, `ApprovalPolicy`, `ApprovalRole`, `ChainPolicy`, `CollectionOptions`, `LoopOptions`, `OperationDisclosure`, `QueryDefinition`, `StateDefinition`, `SwitchCase`, `UpdateDefinition`, `WorkflowLimits`. [ApprovalOptions.java:ApprovalOptions] [ApprovalPolicy.java:ApprovalPolicy] [ApprovalRole.java:ApprovalRole] [ChainPolicy.java:ChainPolicy] [CollectionOptions.java:CollectionOptions] [LoopOptions.java:LoopOptions] [OperationDisclosure.java:OperationDisclosure] [QueryDefinition.java:QueryDefinition] [StateDefinition.java:StateDefinition] [SwitchCase.java:SwitchCase] [UpdateDefinition.java:UpdateDefinition] [WorkflowLimits.java:WorkflowLimits]

## Invariants

- Workflow names use lowercase letters, digits and dashes with length 3–64; `build` requires a nonempty top-level flow and valid local references. [WorkflowBuilder.java:NAME] [WorkflowBuilder.java:build] [WorkflowBuilder.java:validateScope]
- IDs remain unique across nested joined flows; a flow cannot join itself. Named bodies require exactly one return and exclude `end` and `continueAsNew`. Continuation is top-level and requires bounded chain policy; body-reference cycles are rejected. [Flow.java:requireFree] [Flow.java:joinable] [WorkflowBuilder.java:build] [WorkflowBuilder.java:visitBody]
- State definitions cap queries/updates at 32 each and expressions at 1024 UTF-8 bytes, require declared-business-result-v1 disclosure, and validate closed bounded result schemas. Database nodes validate connection names, statements, typed parameters and field ceilings locally. [StateDefinitionValidator.java:build] [StateDefinitionValidator.java:expression] [StateDefinitionValidator.java:resultSchema] [DatabaseOptions.java:toNode] [DatabaseOptions.java:CEILINGS]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
