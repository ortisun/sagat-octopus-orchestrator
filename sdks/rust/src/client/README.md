# client

## Overview

REST extensions for approvals, run-input blobs, reruns, continuation chains and the feature 018 surface (schedules, business identifiers, batch jobs, the with-start calls and resets). [approvals.rs:ApprovalDecisionRequest] [blobs.rs:UploadRunInputOptions] [chains.rs:ChainCommand] [schedules.rs:Schedules] [batch.rs:BatchOperations] [business.rs:BusinessIds] [resets.rs:ResetRequest]

## Interface

- ApprovalDecisionRequest and Client list_approvals/list_all_approvals/decide_approval support approval APIs. UploadRunInputOptions and start_with_blob/rerun_as_new/upload_run_input support staged inputs. [approvals.rs:ApprovalDecisionRequest] [approvals.rs:list_approvals] [approvals.rs:decide_approval] [blobs.rs:UploadRunInputOptions] [blobs.rs:start_with_blob] [blobs.rs:rerun_as_new] [blobs.rs:upload_run_input]
- ChainCommand, ChainQueryRequest, ChainUpdateRequest and Client inspect/list/query/update/signal/cancel/wait methods support chain operations. [chains.rs:ChainCommand] [chains.rs:ChainQueryRequest] [chains.rs:ChainUpdateRequest] [chains.rs:wait_for_chain_completion]
- Client schedules()/batch()/business_ids() return borrowed handles (Schedules, BatchOperations, BusinessIds); signal_with_start/update_with_start, reset_points/preview_reset/reset/list_resets/get_reset are Client methods. Request types (ScheduleWrite, BatchRequest, ResetRequest) are serde types; policies are open enums with an Other(String) variant; Moment is an instant given as a date-time or RFC 3339 text. [schedules.rs:Schedules] [batch.rs:BatchOperations] [business.rs:BusinessIds] [business.rs:signal_with_start] [business.rs:update_with_start] [resets.rs:reset] [wire.rs:Moment] [wire.rs:wire_enum]

## Invariants

- Pagers reject repeated non-null continuation cursors. Decisions and chain requests validate IDs, names, keys and revision bounds before I/O. [approvals.rs:parse] [chains.rs:chain_page] [approvals.rs:decide_approval] [chains.rs:update_chain]
- Every mutating 018 call carries one Idempotency-Key that all retries keep; ids that become path segments must be canonical UUIDs, expected revisions and reset sequences at least 1, and a business identifier is sent as given (only a value that is not 1 to 200 visible ASCII characters, or is exactly . or .., is refused by business_ids().get). Answers keep unknown enumeration values as strings and the whole JSON in raw. [wire.rs:id_segment] [wire.rs:positive_revision] [business.rs:get] [wire.rs:Fields]
- A start delay is a Duration truncated once to integer milliseconds before the first attempt; start accepts HTTP 200 only with the USE_EXISTING policy and start_with_blob refuses a business identifier or delay. [wire.rs:delay_millis] [client.rs:start] [blobs.rs:start_with_blob]
- Uploads consume AsyncRead once and require EOF plus measured byte count and SHA-256 matching the returned descriptor and caller expectations. [blobs.rs:upload_once]

## Side Effects

- Methods perform authenticated REST I/O; uploads consume/hash streams and chain waits poll with jitter and cancellation. [blobs.rs:upload_once] [chains.rs:wait_for_chain_completion]

## Verification

Run from the repository root:

```sh
cd sdks/rust && cargo test --locked
```

Use a private `CARGO_TARGET_DIR` outside the repository, as required by [`sdks/AGENTS.md`](../../../AGENTS.md).

These commands are documented from the existing build configuration; they were not executed during this documentation update.
