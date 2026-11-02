# workflow

## Overview

Local workflow IR authoring, including control flow, state and logical database nodes. [workflow.go:Builder] [state.go:StateDefinition] [database.go:DatabaseOptions]

## Interface

- New and Flow create builders; metadata/schema/limit/chain-policy methods configure documents. Node methods author steps, scripts, sleeps, event waits, children, condition/switch, collections/loops, approvals, continuation, parallel/saga, end/return and named bodies. Build, JSON and Err expose results. [workflow.go:New] [workflow.go:Flow] [workflow.go:Builder] [workflow.go:Build] [workflow.go:JSON] [workflow.go:Err]
- RetryOption, StepOption, ScriptOption, WaitOption, LoopOption, ContinuationOption, ChildOption, SagaOption and Trigger describe node behavior. StateDefinition, QueryDefinition, UpdateDefinition and DeclaredBusinessResultV1 describe state authoring. ConnectionRef, DatabaseValue, DatabaseParameter and DatabaseOptions describe logical SQL inputs. [options.go:RetryOption] [options.go:StepOption] [options.go:ScriptOption] [options.go:WaitOption] [options.go:LoopOption] [options.go:ContinuationOption] [options.go:ChildOption] [options.go:SagaOption] [options.go:Trigger] [state.go:StateDefinition] [state.go:QueryDefinition] [state.go:UpdateDefinition] [state.go:DeclaredBusinessResultV1] [database.go:ConnectionRef] [database.go:DatabaseValue] [database.go:DatabaseParameter] [database.go:DatabaseOptions]

## Invariants

- The first local validation failure is retained; subsequent node pushes are inert. Build checks scopes/body references/cycles and returns fresh copies. [workflow.go:fail] [workflow.go:push] [workflow.go:Build] [workflow.go:clone]
- JSON uses canonical UTF-16 key ordering and safe-number constraints. State/result schemas and database parameters are locally bounded and cloned; these builders emit definitions and do not establish server acceptance or execute SQL. [../internal/jsonx/canonical.go:Canonical] [state.go:resultSchema] [database.go:databaseParameter] [database.go:Database]

## Side Effects

- None

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
