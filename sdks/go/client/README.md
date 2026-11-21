# client

## Overview

Namespace-scoped REST client for runs, approvals, declared state, chains, events and run-input blobs. [client.go:Client] [runs.go:UploadRunInput]

## Interface

- New and WithTimeout/WithRetry/WithHTTPClient/WithTLSConfig/WithUserAgent/WithTraceContext/WithLogger configure transport and identity. [client.go:New] [client.go:WithTimeout] [client.go:WithRetry] [client.go:WithHTTPClient] [client.go:WithTLSConfig] [client.go:WithUserAgent] [client.go:WithTraceContext] [client.go:WithLogger]
- Run APIs: ResolveWorkflow, Start, UploadRunInput, StartWithBlob, Get, List, ListAll, Operate, Cancel, RerunAsNew, Signal, Events and WaitForCompletion. Approval/state APIs: ListApprovals, ListAllApprovals, DecideApproval, Query, QueryChain, Update and UpdateChain. [runs.go:ResolveWorkflow] [runs.go:Start] [runs.go:UploadRunInput] [runs.go:ListAll] [runs.go:Operate] [runs.go:RerunAsNew] [runs.go:WaitForCompletion] [runs.go:ListApprovals] [runs.go:DecideApproval] [runs.go:Query] [runs.go:Update]
- Feature 018: `Client.Start` answers a StartedRun (WithBusinessID, WithBusinessIDConflictPolicy, WithBusinessIDReusePolicy, WithStartDelay); SignalWithStart and UpdateWithStart start the run of a business identifier when none is open; ResetPoints, PreviewReset, Reset, ListResets and GetReset reset a run that ended. The groups `Client.Schedules`, `Client.Batch` and `Client.BusinessIDs` take the namespace by id. Page is the generic page; an enumerated value of an answer is kept as the string it came as. [runs.go:Start] [withstart.go:SignalWithStart] [withstart.go:UpdateWithStart] [reset.go:Reset] [schedules.go:Schedules] [batch.go:BatchOperations] [businessids.go:BusinessIDs] [wire.go:Page]
- Chain APIs: InspectChain, ListChainRuns, ListAllChainRuns, SignalChain, CancelChain and WaitForChainCompletion. Response models retain additive raw fields. [runs.go:InspectChain] [runs.go:ListAllChainRuns] [runs.go:SignalChain] [runs.go:CancelChain] [runs.go:WaitForChainCompletion] [run.go:Raw]

## Invariants

- Each attempt obtains a bearer token and sends namespace. URLs accept http/https only, redirects are refused and plaintext warnings show only safe host information. [transport.go:once] [client.go:New]
- Every call of feature 018 that changes something carries one Idempotency-Key (given, or generated once and reused by its retries); a start delay is turned into whole milliseconds once, before the first attempt; a business identifier is sent as given and percent-encoded in a path, and `.` or `..` is refused; ids in a path are UUIDs, revisions and sequences are at least 1, notes are not empty, and a local refusal sends nothing. [wire.go:mutation] [withstart.go:startDelayMs] [wire.go:escapeSegment] [wire.go:idSegment]
- Only marked repeatable operations retry with stable encoded bytes/keys; raw one-pass uploads are not retried. UUID paths, segments, keys, revisions and traceparents are checked before I/O. [transport.go:request] [transport.go:do] [runs.go:UploadRunInput] [runs.go:runPath] [runs.go:chainPath] [runs.go:segment] [runs.go:key]
- Responses cap at 32 MiB; error fields/waits are bounded and unsupported routes map to NOT_FOUND/UNSUPPORTED_BY_SERVER. [client.go:maxAnswer] [client.go:readAll] [transport.go:errorOf]

## Side Effects

- Performs HTTP requests, consumes uploads, closes responses/idle connections, caches workflow resolutions for a minute, emits safe retry/plaintext logs and uses wait timers. [transport.go:once] [client.go:Close] [runs.go:ResolveWorkflow] [runs.go:WaitForCompletion] [transport.go:do]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
