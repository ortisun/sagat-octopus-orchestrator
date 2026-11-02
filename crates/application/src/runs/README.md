# crates/application/src/runs

## Overview

Admission for retained ad-hoc managed-database runs and fixed connection probes, including provenance, authorization, validation, idempotency and denial audits.

## Interface

- Timeout constants (default 300000 ms, maximum 3600000 ms), `RunOrigin` and `AdHocMode` define admission/provenance vocabulary. [ad_hoc.rs:DEFAULT_AD_HOC_TIMEOUT_MS] [ad_hoc.rs:MAX_AD_HOC_TIMEOUT_MS] [ad_hoc.rs:RunOrigin] [ad_hoc.rs:AdHocMode]
- `StartDatabaseExecution` has redacted Debug; `RetainedDefinition` exposes canonical document/digest/origin/workflow-limit metadata, and `ConnectionProbe` holds safe probe evidence. [ad_hoc.rs:StartDatabaseExecution] [ad_hoc.rs:RetainedDefinition] [ad_hoc.rs:ConnectionProbe]
- Public `RunService` methods start database runs, probe connections and read latest probes; private rerun and crate-private `audit_denied` support those paths. [ad_hoc.rs:RunService::start_database] [ad_hoc.rs:RunService::probe_connection] [ad_hoc.rs:RunService::latest_connection_probe] [ad_hoc.rs:RunService::rerun_private] [ad_hoc.rs:audit_denied]

## Invariants

- Admission requires RunStart, DatabaseAdHoc and DatabaseUse, plus DatabaseWrite for Execute operations; denials are audited. [ad_hoc.rs:RunService::start_private]
- Input is a JSON object bounded by `MAX_PAYLOAD_BYTES`; timeout is 1..=3600000 ms and the idempotency key is 8..=160 ASCII-graphic bytes. [ad_hoc.rs:RunService::start_private]
- Definitions have no triggers, 1..=64 nodes and at least one database operation; non-workflow origins require exactly one database node. Other admitted kinds are timer, condition, parallel and end. [ad_hoc.rs:validate_definition]
- Probes use a fixed SELECT 1 document, empty-object input and exactly 30000 ms. Private reruns require the original initiating subject and retain stored origin/document. [ad_hoc.rs:probe_document] [ad_hoc.rs:RunService::start_private] [ad_hoc.rs:RunService::rerun_private]

## Side Effects

- Namespace/quota/definition-cache/run-store ports validate admission and request atomic retained-definition/run/audit/idempotency creation; read paths retrieve private definitions and probe evidence. [ad_hoc.rs:RunService::start_private] [ad_hoc.rs:RunService::latest_connection_probe] [ad_hoc.rs:RunService::rerun_private]
- Admission/denial paths emit metrics/spans and best-effort denial audits without definition, input, connection or provider contents. [ad_hoc.rs:RunService::start_private] [ad_hoc.rs:audit_denied]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-application --locked
```

These commands were not run in this documentation-only pass.
