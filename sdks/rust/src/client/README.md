# client

## Overview

REST extensions for approvals, run-input blobs, reruns and continuation chains. [approvals.rs:ApprovalDecisionRequest] [blobs.rs:UploadRunInputOptions] [chains.rs:ChainCommand]

## Interface

- ApprovalDecisionRequest and Client list_approvals/list_all_approvals/decide_approval support approval APIs. UploadRunInputOptions and start_with_blob/rerun_as_new/upload_run_input support staged inputs. [approvals.rs:ApprovalDecisionRequest] [approvals.rs:list_approvals] [approvals.rs:decide_approval] [blobs.rs:UploadRunInputOptions] [blobs.rs:start_with_blob] [blobs.rs:rerun_as_new] [blobs.rs:upload_run_input]
- ChainCommand, ChainQueryRequest, ChainUpdateRequest and Client inspect/list/query/update/signal/cancel/wait methods support chain operations. [chains.rs:ChainCommand] [chains.rs:ChainQueryRequest] [chains.rs:ChainUpdateRequest] [chains.rs:wait_for_chain_completion]

## Invariants

- Pagers reject repeated non-null continuation cursors. Decisions and chain requests validate IDs, names, keys and revision bounds before I/O. [approvals.rs:parse] [chains.rs:chain_page] [approvals.rs:decide_approval] [chains.rs:update_chain]
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
