# Workflow primitives: operating and validating the current build

The public workflow document remains additive `sagat-octopus-orchestrator.io/v1`. Bounded
`switch` is the available runtime primitive. Collection, loop, approval, declared operations and
continuation have private models or storage in progress; their builder fixtures and private
integration tests do not authorize publishing those nodes. The publication parser must keep
rejecting them until each primitive's fault, security, compatibility and recovery gates pass.

## Check the running system

Run the contract and PostgreSQL suites from the repository root with the pinned toolchain:

```sh
cargo test -p sagat-tests --test contract_workflow_primitives --locked
cargo test -p sagat-tests --test integration_workflow_primitives --locked
cargo test -p sagat-tests --test fault_workflow_primitives --locked
```

The PostgreSQL test harness starts real infrastructure or fails if it cannot. A passing private
transaction test shows the asserted journal/projection/outbox boundary only; it is not evidence of
a public workflow round trip. See
[`validation-report.md`](../../specs/014-workflow-primitives/validation-report.md) for commands,
counts and remaining gates from the latest run.

For authoring parity, run the shared five-language SDK scenarios individually:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

These scenarios compare complete normalized documents with reviewed goldens; they do not call the
server's runtime APIs. Do not replace a golden merely because a builder output changed: inspect
the contract and the intended compatibility behavior first.

## Diagnose a stuck or duplicated transition

Identify the tenant, namespace, run, workflow version, instance path and last journal sequence.
Inspect the run journal and projection under the same tenant scope; keep the trace ID and request
ID, but omit payloads, credentials and decision policy values from logs. The journal is the
authority. A mismatch with a step, timer, subscription, collection, loop or approval projection
means the transition must not be retried blindly. Preserve the run and investigate the failed
transaction or projection verifier.

For a worker task, compare the attempt and lease fence as well as the instance path. An old
worker minor must never receive a scoped task. A stale completion should leave journal, quota and
outbox unchanged. For a delayed loop or approval, verify that the due record is durable and no
worker lease is retained while waiting; duplicate delivery must produce one committed result.
Broker delivery is a hint, so use reconciliation of PostgreSQL state rather than assuming the
broker delivered exactly once. External business effects require stable idempotency keys or a
reconciliation procedure; neither a journal event nor an SDK retry makes an external effect
exactly once.

For declared Update, check the operation name, immutable definition version, expected state
revision, request hash and update key. A durable rejected receipt must replay the same business
result after the caller's current authorization is rechecked. Query reads one committed snapshot
and adds an access audit, without appending a workflow event or scheduling work. Do not expose
these operations until schema evaluation, role checks and malicious-handler tests pass.

## Rollback and compatibility

Disable publication and starts of any newly enabled primitive before rolling back its writer.
Keep readers, journal codecs, projections and worker capability gates for already active runs
until they drain. Do not remove migrations or delete chain/receipt rows while an active run or
retained idempotency receipt references them. Revert only after validating old active runs, old
worker minors and all five SDKs against the target build. The current PostgreSQL worker adapter
advertises minor 1.4 because scoped claim/report transactions are not yet enabled; its capability
gate must remain closed until those transactions and recovery tests pass.
