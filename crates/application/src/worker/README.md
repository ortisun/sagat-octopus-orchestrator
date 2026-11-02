# crates/application/src/worker

## Overview

Worker use cases for managed-database capabilities, fenced completion, exact-path scoped claims/reports, secret handoff/scanning and durable output refusal. Leaf modules are private; public service methods and BlobOutput are exposed through the parent module.

## Interface

- Public managed-database methods are `resolve_database`, `database_blob_limit` and `check_database_blob_upload`; snapshot/scan helpers remain parent-private. [database.rs:WorkerService::resolve_database] [database.rs:WorkerService::database_blob_limit] [database.rs:WorkerService::check_database_blob_upload]
- Public completion methods `complete` and `complete_with_blob` accept inline or `BlobOutput` reports. [output.rs:BlobOutput] [output.rs:WorkerService::complete] [output.rs:WorkerService::complete_with_blob]
- Public `resolve_secrets` repeats authority checks around provider resolution; scan/report helpers remain parent-private. [secrets.rs:WorkerService::resolve_secrets] [secrets.rs:WorkerService::secret_in_output] [secrets.rs:WorkerService::checked_secret_report]
- Parent-private exact-path helpers open current paths, check fences/bindings, heartbeat, authorize blobs and claim scoped tasks. [scoped.rs:WorkerService::open_worker_path] [scoped.rs:WorkerService::live_path_fence] [scoped.rs:WorkerService::heartbeat_path] [scoped_capabilities.rs:WorkerService::check_path_blob_access] [scoped_claims.rs:WorkerService::claim_run]
- Parent-private `PathReport` handling supports completion/failure, reconciliation, durable receipts, secret scanning and quota fallback; `refuse_scoped_output` performs the full-run refusal path. [scoped_reports.rs:PathReport] [scoped_reports.rs:WorkerService::report_path] [scoped_output_refusal.rs:WorkerService::refuse_scoped_output]
- Parent-private `SecretResolutionTx` unifies root/scoped transaction operations without putting provider values into its transaction interface. [secret_resolution.rs:SecretResolutionTx] [secret_resolution.rs:WorkerService::open_secret_resolution]

## Invariants

- Reports check/replay idempotency receipts before mutation and verify current lease attempt/generation plus applicable stop/cancellation fences. [output.rs:WorkerService::complete_inner] [scoped_reports.rs:WorkerService::checked_path_report] [scoped_output_refusal.rs:WorkerService::strict_refusal_fence]
- Managed-database execution is root-only. Credential resolution/output scanning release transactions around provider work, then reopen, repeat capability checks and require unchanged snapshots. [database.rs:WorkerService::resolve_database_inner] [database.rs:WorkerService::check_database_blob_upload]
- Inline completion must be JSON and at most 256 KiB; retained run output is capped at 4 MiB. Database writes can become outcome-unknown when success cannot safely be disproved. [output.rs:WorkerService::complete_inner]
- Scoped claiming requires adapter support and protocol minor 1.5+, exact path identity, current authorization and activity binding; scoped managed-database tasks are refused. [scoped_claims.rs:WorkerService::claim_run] [scoped.rs:WorkerService::ensure_path_bound]
- Secret handoff is all-or-nothing and repeats pool/publisher ACL/rotation/fence checks. Output scans use retained versions and ignore values shorter than eight bytes or non-UTF-8 values. [secrets.rs:WorkerService::resolve_secrets] [secrets.rs:WorkerService::secret_in_output]

## Side Effects

- Store transaction ports persist events, outputs, leases, receipts and audit effects and commit; these are application-port calls. [output.rs:WorkerService::complete_inner] [scoped_reports.rs:WorkerService::finish_path_report] [scoped_claims.rs:WorkerService::claim_path]
- Configured vault, credential-codec and result-validator ports run outside run locks before authority is revalidated. [database.rs:WorkerService::resolve_database_inner] [database.rs:WorkerService::scan_database_output] [secrets.rs:WorkerService::resolve_secrets]
- Capability denial, secret resolution/leak detection, quota refusal and scoped-output refusal produce metrics, spans and audit effects. [database.rs:WorkerService::resolve_database] [output.rs:WorkerService::reject_secret_output] [scoped_output_refusal.rs:WorkerService::refuse_scoped_output] [secrets.rs:WorkerService::resolve_secrets]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-application --locked
```

These commands were not run in this documentation-only pass.
